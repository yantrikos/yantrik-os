"""LibreOffice, reached over UNO: the one file in this adapter that talks to `uno`.

LibreOffice cannot host a surface the way Blender hosts its add-on — it has no Python of ours
running inside it — so this adapter is a separate process that drives it from outside, over the
UNO remote bridge: LibreOffice started with `--accept=pipe,name=<pipe>;urp;` listens on a pipe,
and this file connects to it, finds the Desktop, and reads and changes documents through the
public UNO API (`com.sun.star.text`, `com.sun.star.sheet`, `com.sun.star.frame`).

Everything here raises `OfficeError` with a sentence a caller can act on, never a UNO exception:
the surface hands the sentence to the caller as the refusal. The `uno` module is imported when it
is first needed, and can be handed in instead (`Office(uno_module=...)`), which is how the tests
drive all of this against a fake.
"""

import json
import os
import re
import zlib

# The pipe LibreOffice listens on when the desktop opens it (bin/yantrik-libreoffice) and this
# adapter connects to. `YANTRIK_LIBREOFFICE_PIPE` changes both.
PIPE = "yantrik-libreoffice"

WRITER, CALC, IMPRESS, DRAW = "writer", "calc", "impress", "draw"
# Impress before Draw: a presentation is a drawing document too.
_SERVICES = (
    ("com.sun.star.text.TextDocument", WRITER),
    ("com.sun.star.sheet.SpreadsheetDocument", CALC),
    ("com.sun.star.presentation.PresentationDocument", IMPRESS),
    ("com.sun.star.drawing.DrawingDocument", DRAW),
)
KIND_NAMES = {WRITER: "a Writer document", CALC: "a Calc spreadsheet",
              IMPRESS: "an Impress presentation", DRAW: "a Draw drawing"}

PDF_FILTERS = {WRITER: "writer_pdf_Export", CALC: "calc_pdf_Export",
               IMPRESS: "impress_pdf_Export", DRAW: "draw_pdf_Export"}

# What `save_as` writes, by the extension of the path it is given: the kind of document that
# can be saved that way, and LibreOffice's filter for it.
SAVE_FILTERS = {
    ".odt": (WRITER, "writer8"),
    ".docx": (WRITER, "MS Word 2007 XML"),
    ".doc": (WRITER, "MS Word 97"),
    ".rtf": (WRITER, "Rich Text Format"),
    ".txt": (WRITER, "Text"),
    ".ods": (CALC, "calc8"),
    ".xlsx": (CALC, "Calc MS Excel 2007 XML"),
    ".xls": (CALC, "MS Excel 97"),
    ".csv": (CALC, "Text - txt - csv (StarCalc)"),
    ".odp": (IMPRESS, "impress8"),
    ".pptx": (IMPRESS, "Impress MS PowerPoint 2007 XML"),
    ".odg": (DRAW, "draw8"),
}

# The most cells one read or one write carries: enough for any table a mind should be reading
# whole, small enough that an answer stays an answer.
MAX_CELLS = 10000
# Past this many cells a sheet's content is not fingerprinted on every describe.
FINGERPRINT_CELLS = 50000

_CELL = re.compile(r"\$?([A-Za-z]{1,3})\$?([1-9][0-9]{0,6})")


class OfficeError(Exception):
    """A sentence for the caller: what went wrong and what to do instead."""


class NotReachable(OfficeError):
    """LibreOffice cannot be reached at all. `short` is the half-line describe's summary uses."""

    def __init__(self, message, short):
        super().__init__(message)
        self.short = short


def uno_error_name(error):
    """The UNO type of an exception, unqualified (`NoConnectException`), however pyuno named the
    Python class it made for it."""
    for owner in (type(error), error):
        name = getattr(owner, "__pyunostruct__", None) or getattr(owner, "typeName", None)
        if isinstance(name, str) and name:
            return name.rsplit(".", 1)[-1]
    return type(error).__name__.rsplit(".", 1)[-1]


def uno_message(error):
    """What a UNO exception said, or its type when it said nothing."""
    message = getattr(error, "Message", None)
    if isinstance(message, str) and message.strip():
        return message.strip()
    text = str(error).strip()
    return text or uno_error_name(error)


def column_name(index):
    """0 → A, 25 → Z, 26 → AA."""
    name = ""
    index += 1
    while index:
        index, rest = divmod(index - 1, 26)
        name = chr(ord("A") + rest) + name
    return name


def cell_position(ref):
    """`B3` → (1, 2): column and row from 0. None for anything that is not one cell."""
    match = _CELL.fullmatch(ref.strip()) if isinstance(ref, str) else None
    if not match:
        return None
    column = 0
    for letter in match.group(1).upper():
        column = column * 26 + (ord(letter) - ord("A") + 1)
    return column - 1, int(match.group(2)) - 1


def range_name(left, top, right, bottom):
    first = "%s%d" % (column_name(left), top + 1)
    last = "%s%d" % (column_name(right), bottom + 1)
    return first if first == last else "%s:%s" % (first, last)


def parse_range(text):
    """`A1:C10` or `B3` → (left, top, right, bottom), or None."""
    parts = text.split(":") if isinstance(text, str) else []
    if len(parts) not in (1, 2):
        return None
    ends = [cell_position(p) for p in parts]
    if any(e is None for e in ends):
        return None
    (c0, r0), (c1, r1) = ends[0], ends[-1]
    return min(c0, c1), min(r0, r1), max(c0, c1), max(r0, r1)


def absolute(path, what="`path`"):
    """A path a caller gave, `~` expanded; refused unless it is absolute, because this adapter's
    working directory is nobody's business and a relative path would land wherever it is."""
    if not isinstance(path, str) or not path.strip():
        raise OfficeError("%s is empty; give an absolute path" % what)
    expanded = os.path.expanduser(path.strip())
    if not os.path.isabs(expanded):
        raise OfficeError("%s must be an absolute path (or start with ~), and `%s` is not"
                          % (what, path.strip()))
    return os.path.normpath(expanded)


def _number(value):
    """A cell's number as JSON would like it: 3 for 3.0."""
    if isinstance(value, float) and value.is_integer() and abs(value) < 2 ** 53:
        return int(value)
    return value


class Office:
    """One LibreOffice, reached over the pipe `pipe`.

    Connected lazily and again after LibreOffice goes away and comes back: every public method
    connects if it has to, and raises `OfficeError` saying LibreOffice is not there when it is
    not. `hidden` opens documents without a window (for the adapter's own headless LibreOffice).
    """

    def __init__(self, pipe=PIPE, uno_module=None, hidden=False, admit=None):
        self.pipe = pipe
        self.hidden = hidden
        self._uno = uno_module
        self._desktop = None
        # `admit(what, path)`: the rule for the files a caller names ("read", "write") and for
        # the document a call lands on ("document", its file or None), raising to refuse. The
        # surface sets it (surface.py); left None, every file is the caller's to name.
        self.admit = admit

    def _admit(self, what, path):
        if self.admit is None:
            return
        try:
            self.admit(what, path)
        except OfficeError:
            raise
        except Exception as e:  # noqa: BLE001 - the rule's refusal is the caller's sentence
            raise OfficeError(str(e)) from None

    # ── the connection ──────────────────────────────────────────────────────

    def connect_string(self):
        return "uno:pipe,name=%s;urp;StarOffice.ComponentContext" % self.pipe

    def uno(self):
        if self._uno is None:
            try:
                import uno  # noqa: PLC0415 - only a machine with python3-uno has it
            except ImportError:
                raise NotReachable("this adapter cannot reach LibreOffice: its Python has no "
                                   "`uno` module (on Debian, the python3-uno package)",
                                   "no python3-uno to reach it with") from None
            self._uno = uno
        return self._uno

    def desktop(self):
        """The Desktop of the LibreOffice on the pipe, connecting if need be.

        A connection kept from an earlier call is asked one harmless question first: a
        LibreOffice that quit (and perhaps started again) since then left a dead bridge behind,
        and every call through it would fail. So the dead one is dropped and a new one made
        before any work is done — work is never retried, because a write done twice is not the
        write that was asked for."""
        if self._desktop is not None:
            try:
                self._desktop.getCurrentComponent()
                return self._desktop
            except Exception:  # noqa: BLE001 - whatever it was, that bridge is no use
                self._desktop = None
        uno = self.uno()
        try:
            local = uno.getComponentContext()
            resolver = local.ServiceManager.createInstanceWithContext(
                "com.sun.star.bridge.UnoUrlResolver", local)
        except Exception as e:  # noqa: BLE001 - a UNO runtime that will not start is a sentence
            raise NotReachable("this adapter's UNO runtime would not start: %s" % uno_message(e),
                               "no UNO runtime to reach it with") from None
        try:
            context = resolver.resolve(self.connect_string())
        except Exception as e:  # noqa: BLE001 - every UNO failure becomes a sentence
            if uno_error_name(e) in ("NoConnectException", "ConnectionSetupException"):
                raise NotReachable(
                    "LibreOffice is not running, or not listening on the pipe `%s` — open it from "
                    "the desktop, which starts it listening, or start it with "
                    "--accept=\"pipe,name=%s;urp;\"" % (self.pipe, self.pipe),
                    "not running") from None
            raise NotReachable("LibreOffice would not connect: %s" % uno_message(e),
                               "would not connect") from None
        self._desktop = context.ServiceManager.createInstanceWithContext(
            "com.sun.star.frame.Desktop", context)
        return self._desktop

    def forget(self):
        """Drop the connection; the next call makes a new one."""
        self._desktop = None

    def run(self, work):
        """`work(desktop)`, once, on a live connection; UNO failures become sentences."""
        desktop = self.desktop()
        try:
            return work(desktop)
        except OfficeError:
            raise
        except Exception as e:  # noqa: BLE001 - every UNO failure becomes a sentence
            name = uno_error_name(e)
            if name in ("DisposedException", "RuntimeException"):
                self.forget()
            if name == "DisposedException":
                raise OfficeError("LibreOffice stopped answering, or the document was closed "
                                  "while this ran; describe it again before deciding") from None
            raise OfficeError("LibreOffice refused: %s" % uno_message(e)) from None

    def _prop(self, name, value):
        prop = self.uno().createUnoStruct("com.sun.star.beans.PropertyValue")
        prop.Name = name
        prop.Value = value
        return prop

    # ── documents ───────────────────────────────────────────────────────────

    @staticmethod
    def kind(component):
        """writer, calc, impress or draw — or None for a component that is not a document of
        a kind this adapter knows (the Start Center, a database, a formula)."""
        supports = getattr(component, "supportsService", None)
        if supports is None:
            return None
        for service, kind in _SERVICES:
            try:
                if supports(service):
                    return kind
            except Exception:  # noqa: BLE001 - a component that will not say is not ours
                return None
        return None

    def path_of(self, doc):
        url = doc.getURL() or ""
        return self.uno().fileUrlToSystemPath(url) if url.startswith("file:") else None

    def name_of(self, doc):
        """What a caller calls a document: its file's name, or LibreOffice's title for one that
        has never been saved (`Untitled 1`)."""
        path = self.path_of(doc)
        if path:
            return os.path.basename(path)
        try:
            return doc.getTitle()
        except Exception:  # noqa: BLE001 - a model without XTitle
            return "Untitled"

    def _documents(self, desktop):
        found = []
        enumeration = desktop.getComponents().createEnumeration()
        while enumeration.hasMoreElements():
            component = enumeration.nextElement()
            if self.kind(component) is not None:
                found.append(component)
        return found

    def _front(self, desktop, docs):
        """The document in front, if it is one of `docs`."""
        try:
            current = desktop.getCurrentComponent()
        except Exception:  # noqa: BLE001 - no frame in front
            return None
        if current is None:
            return None
        for doc in docs:
            if doc == current:
                return doc
        return None

    def _find(self, desktop, which):
        """The document `which` names, once the rule has had its say about it: every action on
        a document finds it here, so none reads or changes one the caller may not."""
        doc = self._pick(desktop, which)
        self._admit("document", self.path_of(doc))
        return doc

    def _pick(self, desktop, which):
        docs = self._documents(desktop)
        if not docs:
            raise OfficeError("no document is open in LibreOffice; `open` one first")
        names = [self.name_of(d) for d in docs]
        if which is None or not str(which).strip():
            if len(docs) == 1:
                return docs[0]
            front = self._front(desktop, docs)
            if front is not None:
                return front
            raise OfficeError("%d documents are open (%s); say which with `document`"
                              % (len(docs), ", ".join(names)))
        which = str(which).strip()
        wanted_path = os.path.normpath(os.path.expanduser(which)) if (
            os.sep in which or which.startswith("~")) else None
        by_path = [d for d in docs if wanted_path and self.path_of(d) == wanted_path]
        if by_path:
            return by_path[0]
        folded = which.casefold()
        by_name = [d for d, n in zip(docs, names) if n.casefold() == folded]
        if len(by_name) == 1:
            return by_name[0]
        if len(by_name) > 1:
            raise OfficeError("`%s` names %d open documents; give its path instead (%s)"
                              % (which, len(by_name),
                                 ", ".join(self.path_of(d) or self.name_of(d) for d in by_name)))
        raise OfficeError("no open document is `%s`; open: %s" % (which, ", ".join(names)))

    def _want(self, doc, kind, instead):
        have = self.kind(doc)
        if have != kind:
            raise OfficeError("`%s` is %s, not %s%s" % (
                self.name_of(doc), KIND_NAMES[have], KIND_NAMES[kind],
                "; %s" % instead if instead else ""))

    def _writable(self, doc):
        if doc.isReadonly():
            raise OfficeError("`%s` is open read-only; nothing can change it. Close it and open "
                              "it again without `read_only`" % self.name_of(doc))

    # ── what describe shows ─────────────────────────────────────────────────

    def overview(self):
        """Everything describe shows, in one read: `(documents, front)`, where each document is
        a dict in this adapter's vocabulary and `front` is the name of the one in front."""
        def read(desktop):
            docs = self._documents(desktop)
            front = self._front(desktop, docs)
            return [self._summary(d) for d in docs], (self.name_of(front) if front else None)
        return self.run(read)

    def _summary(self, doc):
        kind = self.kind(doc)
        out = {"name": self.name_of(doc), "kind": kind, "path": self.path_of(doc),
               "modified": bool(doc.isModified()), "read_only": bool(doc.isReadonly())}
        if kind == WRITER:
            text = doc.getText().getString()
            out["characters"] = len(text)
            out["content"] = "%08x" % zlib.crc32(text.encode("utf-8"))
        elif kind == CALC:
            sheets, digest, cells = [], 0, 0
            for sheet in self._sheets(doc):
                left, top, right, bottom = self._used(sheet)
                sheets.append({"name": sheet.getName(),
                               "used": range_name(left, top, right, bottom)})
                cells += (right - left + 1) * (bottom - top + 1)
                if cells <= FINGERPRINT_CELLS:
                    formulas = sheet.getCellRangeByPosition(left, top, right, bottom) \
                        .getFormulaArray()
                    digest = zlib.crc32(json.dumps([sheet.getName(), formulas]).encode("utf-8"),
                                        digest)
            out["sheets"] = sheets
            # A fingerprint of every cell, so the revision moves when a cell does; a workbook
            # too big to read on every describe says so instead of pretending.
            out["content"] = "%08x" % digest if cells <= FINGERPRINT_CELLS else "too large to fingerprint"
        return out

    # ── actions ─────────────────────────────────────────────────────────────

    def open(self, path, read_only=False):
        path = absolute(path)
        self._admit("read", path)
        if not os.path.isfile(path):
            raise OfficeError("no file at %s" % path)

        def load(desktop):
            url = self.uno().systemPathToFileUrl(path)
            for doc in self._documents(desktop):
                if doc.getURL() == url:
                    return {"document": self.name_of(doc), "kind": self.kind(doc), "path": path,
                            "already_open": True}
            props = [self._prop("ReadOnly", bool(read_only))]
            if self.hidden:
                props.append(self._prop("Hidden", True))
            doc = desktop.loadComponentFromURL(url, "_blank", 0, tuple(props))
            if doc is None:
                raise OfficeError("LibreOffice could not open %s: it did not recognise the file"
                                  % path)
            kind = self.kind(doc)
            if kind is None:
                doc.close(True)
                raise OfficeError("%s opened as something this adapter cannot read or write "
                                  "(not a text document, spreadsheet, presentation or drawing), "
                                  "so it was closed again" % path)
            return {"document": self.name_of(doc), "kind": kind, "path": path,
                    "already_open": False}
        return self.run(load)

    def read_text(self, document=None, start=0, max_chars=20000):
        if start < 0 or max_chars < 1:
            raise OfficeError("`start` must be 0 or more and `max_chars` 1 or more")

        def read(desktop):
            doc = self._find(desktop, document)
            self._want(doc, WRITER, "`read_cells` reads a spreadsheet" if self.kind(doc) == CALC
                       else "")
            text = doc.getText().getString().replace("\r\n", "\n")
            end = min(len(text), start + max_chars)
            return {"document": self.name_of(doc), "characters": len(text), "start": start,
                    "text": text[start:end], "more": end < len(text)}
        return self.run(read)

    def write_text(self, text, at="end", document=None):
        def write(desktop):
            doc = self._find(desktop, document)
            self._want(doc, WRITER, "`write_cells` writes a spreadsheet" if self.kind(doc) == CALC
                       else "")
            self._writable(doc)
            body = doc.getText()
            if at == "replace_all":
                body.setString(text)
            elif at == "start":
                body.insertString(body.getStart(), text, False)
            else:
                body.insertString(body.getEnd(), text, False)
            return {"document": self.name_of(doc), "at": at, "wrote": len(text),
                    "characters": len(body.getString()), "modified": bool(doc.isModified())}
        return self.run(write)

    def _sheets(self, doc):
        sheets = doc.getSheets()
        return [sheets.getByIndex(i) for i in range(sheets.getCount())]

    def _sheet(self, doc, name):
        sheets = doc.getSheets()
        if name is not None and str(name).strip():
            if not sheets.hasByName(name):
                raise OfficeError("`%s` has no sheet `%s`; its sheets: %s"
                                  % (self.name_of(doc), name, ", ".join(sheets.getElementNames())))
            return sheets.getByName(name)
        try:
            controller = doc.getCurrentController()
            active = controller.getActiveSheet() if controller is not None else None
        except Exception:  # noqa: BLE001 - no view (a hidden document): the first sheet
            active = None
        return active if active is not None else sheets.getByIndex(0)

    @staticmethod
    def _used(sheet):
        cursor = sheet.createCursor()
        cursor.gotoStartOfUsedArea(False)
        cursor.gotoEndOfUsedArea(True)
        a = cursor.getRangeAddress()
        return a.StartColumn, a.StartRow, a.EndColumn, a.EndRow

    def read_cells(self, range=None, sheet=None, document=None):  # noqa: A002 - the caller's word
        if range is not None and parse_range(range) is None:
            raise OfficeError("`%s` is not a cell range; write one like `A1:D20`, or one cell "
                              "like `B3`" % range)

        def read(desktop):
            doc = self._find(desktop, document)
            self._want(doc, CALC, "`read_text` reads a text document" if self.kind(doc) == WRITER
                       else "")
            target = self._sheet(doc, sheet)
            left, top, right, bottom = parse_range(range) if range is not None else self._used(target)
            count = (right - left + 1) * (bottom - top + 1)
            if count > MAX_CELLS:
                raise OfficeError("%s is %d cells; read at most %d at a time"
                                  % (range_name(left, top, right, bottom), count, MAX_CELLS))
            cells = target.getCellRangeByPosition(left, top, right, bottom)
            values, formulas = cells.getDataArray(), cells.getFormulaArray()
            rows, behind = [], {}
            for i, row in enumerate(values):
                out = []
                for j, value in enumerate(row):
                    formula = formulas[i][j]
                    if isinstance(value, str) and value == "" and formula == "":
                        value = None
                    out.append(_number(value))
                    # A formula, not text that happens to start with `=`: a text cell's value is
                    # its text, and a formula's value is what it computes.
                    if isinstance(formula, str) and formula.startswith("=") and value != formula:
                        behind["%s%d" % (column_name(left + j), top + i + 1)] = formula
                rows.append(out)
            return {"document": self.name_of(doc), "sheet": target.getName(),
                    "range": range_name(left, top, right, bottom), "rows": rows,
                    "formulas": behind}
        return self.run(read)

    def write_cells(self, cells, sheet=None, document=None):
        if not cells:
            raise OfficeError("`cells` is empty; map each cell to what goes in it, like "
                              "{\"A1\": \"Total\", \"B1\": 42}")
        if len(cells) > MAX_CELLS:
            raise OfficeError("%d cells in one write; write at most %d at a time"
                              % (len(cells), MAX_CELLS))
        # Every cell checked before any is written, so a bad one leaves the sheet as it was.
        plan = []
        for ref, value in sorted(cells.items()):
            position = cell_position(ref)
            if position is None or ":" in ref:
                raise OfficeError("`%s` is not a cell; name one cell, like `B3`" % ref)
            if isinstance(value, bool) or not (value is None or isinstance(value, (int, float, str))):
                raise OfficeError("%s: a cell takes a number, text, a formula starting with `=`, "
                                  "or null to empty it" % ref.upper())
            plan.append((ref.strip().upper().replace("$", ""), position, value))
        plan.sort(key=lambda step: step[0])

        def write(desktop):
            doc = self._find(desktop, document)
            self._want(doc, CALC, "`write_text` writes a text document"
                       if self.kind(doc) == WRITER else "")
            self._writable(doc)
            target = self._sheet(doc, sheet)
            # Every cell found before any is written: one outside the sheet refuses them all.
            found = []
            for ref, (column, row), value in plan:
                try:
                    found.append((target.getCellByPosition(column, row), value))
                except Exception as e:  # noqa: BLE001
                    if uno_error_name(e) == "IndexOutOfBoundsException":
                        raise OfficeError("%s is outside the sheet `%s`"
                                          % (ref, target.getName())) from None
                    raise
            for cell, value in found:
                if value is None:
                    cell.setFormula("")
                elif isinstance(value, str) and value.startswith("="):
                    cell.setFormula(value)
                elif isinstance(value, str):
                    cell.setString(value[1:] if value.startswith("'") else value)
                else:
                    cell.setValue(float(value))
            return {"document": self.name_of(doc), "sheet": target.getName(),
                    "written": [ref for ref, _, _ in plan], "modified": bool(doc.isModified())}
        return self.run(write)

    def save(self, document=None):
        def store(desktop):
            doc = self._find(desktop, document)
            path = self.path_of(doc)
            if not path:
                raise OfficeError("`%s` has never been saved, so it has no file to save over; "
                                  "`save_as` it to a path" % self.name_of(doc))
            self._admit("write", path)
            self._writable(doc)
            doc.store()
            return {"document": self.name_of(doc), "path": path, "modified": bool(doc.isModified())}
        return self.run(store)

    def _new_file(self, path, suffixes):
        """A path to a file that does not exist yet, in a directory that does."""
        path = absolute(path)
        self._admit("write", path)
        if os.path.lexists(path):
            raise OfficeError("a file is already at %s, and this action never replaces one; "
                              "choose another path" % path)
        if not os.path.isdir(os.path.dirname(path)):
            raise OfficeError("there is no directory %s to write into" % os.path.dirname(path))
        if suffixes is not None and os.path.splitext(path)[1].lower() not in suffixes:
            raise OfficeError("%s must end in %s" % (path, " or ".join(sorted(suffixes))))
        return path

    def save_as(self, path, document=None):
        path = self._new_file(path, SAVE_FILTERS)

        def store(desktop):
            doc = self._find(desktop, document)
            kind, filter_name = SAVE_FILTERS[os.path.splitext(path)[1].lower()]
            if self.kind(doc) != kind:
                fits = sorted(ext for ext, (k, _) in SAVE_FILTERS.items() if k == self.kind(doc))
                raise OfficeError("`%s` is %s, which cannot be saved as %s; it can be saved as %s"
                                  % (self.name_of(doc), KIND_NAMES[self.kind(doc)],
                                     os.path.splitext(path)[1], ", ".join(fits)))
            was = self.path_of(doc)
            doc.storeAsURL(self.uno().systemPathToFileUrl(path),
                           (self._prop("FilterName", filter_name), self._prop("Overwrite", False)))
            return {"document": self.name_of(doc), "path": path, "was": was}
        return self.run(store)

    def export_pdf_plan(self, path, document=None):
        """Everything `export_pdf` checks before it writes: `(doc name, work)`, where `work()`
        writes the PDF and returns the answer. Split so the surface can check in its turn and
        write outside it."""
        path = self._new_file(path, {".pdf"})

        def find(desktop):
            doc = self._find(desktop, document)
            return doc, self.name_of(doc), self.kind(doc)
        doc, name, kind = self.run(find)

        def work():
            def store(_desktop):
                doc.storeToURL(self.uno().systemPathToFileUrl(path),
                               (self._prop("FilterName", PDF_FILTERS[kind]),
                                self._prop("Overwrite", False)))
            self.run(store)
            if not os.path.isfile(path):
                raise OfficeError("LibreOffice answered, and nothing is at %s" % path)
            return {"document": name, "path": path, "bytes": os.path.getsize(path)}
        return name, work

    def close(self, document=None):
        def shut(desktop):
            doc = self._find(desktop, document)
            name = self.name_of(doc)
            if doc.isModified():
                raise OfficeError("`%s` has changes that are not saved; `save` or `save_as` it "
                                  "first — this adapter never throws edits away" % name)
            doc.close(True)
            return {"closed": name}
        return self.run(shut)

    def terminate(self):
        """Ask LibreOffice to quit (the adapter's own headless one, on the way out)."""
        try:
            self.run(lambda desktop: desktop.terminate())
        except OfficeError:
            pass
        self.forget()
