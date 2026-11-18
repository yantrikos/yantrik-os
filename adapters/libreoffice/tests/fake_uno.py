"""A stand-in for the slice of LibreOffice's UNO API the adapter uses, and nothing more.

`FakeLibreOffice` is one soffice process: started and stopped, listening on a pipe, holding
documents. `FakeUno(office)` stands in for the `uno` module an adapter imports, and everything
reached through it — the resolver, the Desktop, documents, text, sheets, cells — behaves as the
real API does where the adapter can tell: numbers come back as floats and empty cells as "", a
document that was closed or a LibreOffice that quit raises DisposedException on every call, a
missing pipe raises NoConnectException, `storeAsURL` with Overwrite false refuses a file that
exists.

Documents on disk are JSON — `{"kind": "writer", "text": ...}` or `{"kind": "calc", "sheets":
[{"name": ..., "cells": {"A1": 42, "B1": "text", "C1": "=A1*2"}}]}` — written by `write_doc`
and by the fake's own `store`. A file that is not JSON opens as text in Writer, as LibreOffice
opens a text file. An exported PDF is a small file that starts `%PDF-` and carries the text.
"""

import json
import os
import re
import urllib.parse

WRITER_SERVICE = "com.sun.star.text.TextDocument"
CALC_SERVICE = "com.sun.star.sheet.SpreadsheetDocument"
MAX_COLUMNS, MAX_ROWS = 1024, 1048576


class UnoException(Exception):
    """What pyuno raises: a Python class per UNO type, carrying `Message`."""

    __pyunostruct__ = "com.sun.star.uno.Exception"

    def __init__(self, message=""):
        super().__init__(message)
        self.Message = message


def _uno_exception(qualified):
    return type(qualified.rsplit(".", 1)[-1], (UnoException,), {"__pyunostruct__": qualified})


NoConnectException = _uno_exception("com.sun.star.connection.NoConnectException")
DisposedException = _uno_exception("com.sun.star.lang.DisposedException")
IOException = _uno_exception("com.sun.star.io.IOException")
IllegalArgumentException = _uno_exception("com.sun.star.lang.IllegalArgumentException")
IndexOutOfBoundsException = _uno_exception("com.sun.star.lang.IndexOutOfBoundsException")


def column_index(letters):
    n = 0
    for letter in letters.upper():
        n = n * 26 + ord(letter) - ord("A") + 1
    return n - 1


def cell_key(column, row):
    name, n = "", column + 1
    while n:
        n, rest = divmod(n - 1, 26)
        name = chr(ord("A") + rest) + name
    return "%s%d" % (name, row + 1)


def parse_key(key):
    m = re.fullmatch(r"([A-Z]+)([0-9]+)", key)
    return column_index(m.group(1)), int(m.group(2)) - 1


def write_doc(path, kind, **content):
    """Put a document the fake can open at `path`."""
    doc = {"kind": kind}
    doc.update(content)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(doc, f)


class FakeLibreOffice:
    """One soffice: running or not, on a pipe, with its documents.

    Each `start` is a new process: objects handed out by an earlier one are dead, as they are
    when the real one quits."""

    def __init__(self, pipe="yantrik-libreoffice"):
        self.pipe = pipe
        self.running = False
        self.generation = 0
        self.documents = []
        self.others = []        # components that are not documents (the Start Center)
        self.front = None
        self.connections = 0
        self.untitled = 0
        self.stores = []        # (what, path) for every write to disk

    def start(self):
        self.running = True
        self.generation += 1
        self.documents, self.front = [], None
        self.others = [StartCenter(self)]
        return self

    def stop(self):
        self.running = False

    def new_document(self, kind):
        """A document that has never been saved, as File ▸ New makes one."""
        self.untitled += 1
        doc = (WriterDoc if kind == "writer" else CalcDoc)(self, None, {"kind": kind}, False)
        doc.title = "Untitled %d" % self.untitled
        self.documents.append(doc)
        self.front = doc
        return doc


class _Remote:
    """Anything handed out by one soffice process: dead once that process is."""

    def __init__(self, office):
        self._office = office
        self._generation = office.generation

    def _check(self):
        if not self._office.running or self._generation != self._office.generation:
            raise DisposedException("Binary URP bridge disposed during call")


class StartCenter(_Remote):
    def supportsService(self, name):
        self._check()
        return False


class PropertyValue:
    def __init__(self):
        self.Name, self.Value = "", None


class _Service:
    def __init__(self, make):
        self._make = make

    def createInstanceWithContext(self, name, context):
        return self._make(name, context)


class _LocalContext:
    def __init__(self, office):
        self.ServiceManager = _Service(lambda name, _ctx: Resolver(office) if name ==
                                       "com.sun.star.bridge.UnoUrlResolver" else None)


class Resolver:
    def __init__(self, office):
        self.office = office

    def resolve(self, connect):
        m = re.fullmatch(r"uno:pipe,name=([^;]+);urp;StarOffice\.ComponentContext", connect)
        if not m:
            raise IllegalArgumentException("bad connect string %s" % connect)
        if not self.office.running or m.group(1) != self.office.pipe:
            raise NoConnectException("Connector : couldn't connect to pipe %s" % m.group(1))
        self.office.connections += 1
        return _RemoteContext(self.office)


class _RemoteContext(_Remote):
    def __init__(self, office):
        super().__init__(office)
        self.ServiceManager = _Service(lambda name, _ctx: Desktop(office) if name ==
                                       "com.sun.star.frame.Desktop" else None)


class _Enumeration:
    def __init__(self, items):
        self.items = list(items)

    def hasMoreElements(self):
        return bool(self.items)

    def nextElement(self):
        return self.items.pop(0)


class _EnumerationAccess:
    def __init__(self, items):
        self.items = items

    def createEnumeration(self):
        return _Enumeration(self.items)


class Desktop(_Remote):
    def getComponents(self):
        self._check()
        return _EnumerationAccess(self._office.others + self._office.documents)

    def getCurrentComponent(self):
        self._check()
        return self._office.front

    def loadComponentFromURL(self, url, target, flags, props):
        self._check()
        assert target == "_blank" and flags == 0, (target, flags)
        assert isinstance(props, tuple), "a sequence<PropertyValue> is a tuple in pyuno"
        options = {p.Name: p.Value for p in props}
        path = urllib.parse.unquote(url[len("file://"):])
        if not os.path.isfile(path):
            raise IllegalArgumentException("Unsupported URL <%s>" % url)
        with open(path, encoding="utf-8") as f:
            raw = f.read()
        try:
            content = json.loads(raw)
        except ValueError:
            content = {"kind": "writer", "text": raw}
        kind = content.get("kind")
        if kind not in ("writer", "calc"):
            return None
        doc = (WriterDoc if kind == "writer" else CalcDoc)(
            self._office, url, content, bool(options.get("ReadOnly")))
        doc.hidden = bool(options.get("Hidden"))
        self._office.documents.append(doc)
        self._office.front = doc
        return doc

    def terminate(self):
        self._check()
        self._office.stop()
        return True


class _Doc(_Remote):
    service = None

    def __init__(self, office, url, content, read_only):
        super().__init__(office)
        self.url = url or ""
        self.read_only = read_only
        self.modified = False
        self.title = None
        self.hidden = False
        self.closed = False

    def _check(self):
        super()._check()
        if self.closed:
            raise DisposedException("the document was closed")

    def supportsService(self, name):
        self._check()
        return name == self.service

    def getURL(self):
        self._check()
        return self.url

    def getTitle(self):
        self._check()
        return self.title or os.path.basename(urllib.parse.unquote(self.url))

    def isModified(self):
        self._check()
        return self.modified

    def isReadonly(self):
        self._check()
        return self.read_only

    def _touch(self):
        if self.read_only:
            raise IOException("the document is read-only")
        self.modified = True

    def _write(self, path, overwrite=True):
        if os.path.exists(path) and not overwrite:
            raise IOException("file exists: %s" % path)
        with open(path, "w", encoding="utf-8") as f:
            json.dump(self.content(), f)

    def store(self):
        self._check()
        if not self.url:
            raise IOException("no location")
        if self.read_only:
            raise IOException("read-only")
        path = urllib.parse.unquote(self.url[len("file://"):])
        self._write(path)
        self.modified = False
        self._office.stores.append(("store", path))

    def storeAsURL(self, url, props):
        self._check()
        options = {p.Name: p.Value for p in props}
        path = urllib.parse.unquote(url[len("file://"):])
        self._write(path, overwrite=options.get("Overwrite", True))
        self.url, self.title, self.modified = url, None, False
        self._office.stores.append(("storeAs:%s" % options.get("FilterName"), path))

    def storeToURL(self, url, props):
        self._check()
        options = {p.Name: p.Value for p in props}
        path = urllib.parse.unquote(url[len("file://"):])
        if os.path.exists(path) and not options.get("Overwrite", True):
            raise IOException("file exists: %s" % path)
        if options.get("FilterName", "").endswith("_pdf_Export"):
            with open(path, "wb") as f:
                f.write(b"%PDF-1.7\n% fake PDF\n" + self.pdf_text().encode("utf-8") + b"\n%%EOF\n")
        else:
            self._write(path)
        self._office.stores.append(("storeTo:%s" % options.get("FilterName"), path))

    def close(self, deliver_ownership):
        self._check()
        self.closed = True
        self._office.documents.remove(self)
        if self._office.front is self:
            self._office.front = self._office.documents[-1] if self._office.documents else None

    def getCurrentController(self):
        self._check()
        return _Controller(self)


class _Controller:
    def __init__(self, doc):
        self.doc = doc

    def getActiveSheet(self):
        self.doc._check()
        return self.doc.sheets[self.doc.active] if isinstance(self.doc, CalcDoc) else None


class _TextRange:
    def __init__(self, where):
        self.where = where


class _Text:
    def __init__(self, doc):
        self.doc = doc

    def getString(self):
        self.doc._check()
        return self.doc.text

    def setString(self, text):
        self.doc._check()
        self.doc._touch()
        self.doc.text = text

    def getStart(self):
        self.doc._check()
        return _TextRange("start")

    def getEnd(self):
        self.doc._check()
        return _TextRange("end")

    def insertString(self, at, text, absorb):
        self.doc._check()
        assert absorb is False
        self.doc._touch()
        self.doc.text = text + self.doc.text if at.where == "start" else self.doc.text + text


class WriterDoc(_Doc):
    service = WRITER_SERVICE

    def __init__(self, office, url, content, read_only):
        super().__init__(office, url, content, read_only)
        self.text = content.get("text", "")

    def getText(self):
        self._check()
        return _Text(self)

    def content(self):
        return {"kind": "writer", "text": self.text}

    def pdf_text(self):
        return self.text


class _Names:
    def __init__(self, names):
        self.names = names

    def getElementNames(self):
        return tuple(self.names)


class CalcDoc(_Doc):
    service = CALC_SERVICE

    def __init__(self, office, url, content, read_only):
        super().__init__(office, url, content, read_only)
        self.sheets = [Sheet(self, s["name"], s.get("cells", {}))
                       for s in content.get("sheets", [{"name": "Sheet1"}])]
        self.active = 0
        # What a real Calc document lists once a formula links to another file.
        self.external_links = []

    @property
    def ExternalDocLinks(self):
        return _Names(self.external_links)

    @property
    def DDELinks(self):
        return _Names([])

    def getSheets(self):
        self._check()
        return _Sheets(self)

    def content(self):
        return {"kind": "calc", "sheets": [s.content() for s in self.sheets]}

    def pdf_text(self):
        return "\n".join(str(v) for s in self.sheets for v in s.cells.values())


class _Sheets:
    def __init__(self, doc):
        self.doc = doc

    def getCount(self):
        self.doc._check()
        return len(self.doc.sheets)

    def getByIndex(self, i):
        self.doc._check()
        if not 0 <= i < len(self.doc.sheets):
            raise IndexOutOfBoundsException(str(i))
        return self.doc.sheets[i]

    def hasByName(self, name):
        self.doc._check()
        return any(s.name == name for s in self.doc.sheets)

    def getByName(self, name):
        self.doc._check()
        for s in self.doc.sheets:
            if s.name == name:
                return s
        raise UnoException("NoSuchElementException %s" % name)

    def getElementNames(self):
        self.doc._check()
        return tuple(s.name for s in self.doc.sheets)


class _Address:
    def __init__(self, left, top, right, bottom):
        self.StartColumn, self.StartRow, self.EndColumn, self.EndRow = left, top, right, bottom


class _Cursor:
    def __init__(self, sheet):
        self.sheet = sheet

    def gotoStartOfUsedArea(self, expand):
        self.sheet.doc._check()

    def gotoEndOfUsedArea(self, expand):
        self.sheet.doc._check()

    def getRangeAddress(self):
        self.sheet.doc._check()
        if not self.sheet.cells:
            return _Address(0, 0, 0, 0)
        keys = [parse_key(k) for k in self.sheet.cells]
        return _Address(min(c for c, _ in keys), min(r for _, r in keys),
                        max(c for c, _ in keys), max(r for _, r in keys))


def _stored(value):
    """What a cell given in a fake file holds: a number, text, or a formula."""
    if isinstance(value, str) and value.startswith("="):
        return {"formula": value, "value": 0.0}
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return {"value": float(value)}
    return {"text": str(value)}


class Sheet:
    def __init__(self, doc, name, cells):
        self.doc = doc
        self.name = name
        self.cells = {k: _stored(v) for k, v in cells.items()}

    def content(self):
        out = {}
        for key, cell in self.cells.items():
            out[key] = cell.get("formula") or cell.get("text") or cell.get("value")
        return {"name": self.name, "cells": out}

    def getName(self):
        self.doc._check()
        return self.name

    def createCursor(self):
        self.doc._check()
        return _Cursor(self)

    def getCellRangeByPosition(self, left, top, right, bottom):
        self.doc._check()
        return _Range(self, left, top, right, bottom)

    def getCellByPosition(self, column, row):
        self.doc._check()
        if not (0 <= column < MAX_COLUMNS and 0 <= row < MAX_ROWS):
            raise IndexOutOfBoundsException("%d, %d" % (column, row))
        return _Cell(self, cell_key(column, row))


class _Range:
    def __init__(self, sheet, left, top, right, bottom):
        self.sheet, self.box = sheet, (left, top, right, bottom)

    def _grid(self, read):
        self.sheet.doc._check()
        left, top, right, bottom = self.box
        return tuple(tuple(read(self.sheet.cells.get(cell_key(c, r)))
                           for c in range(left, right + 1)) for r in range(top, bottom + 1))

    def getDataArray(self):
        def value(cell):
            if cell is None:
                return ""
            return cell["text"] if "text" in cell else cell["value"]
        return self._grid(value)

    def getFormulaArray(self):
        def formula(cell):
            if cell is None:
                return ""
            if "formula" in cell:
                return cell["formula"]
            if "text" in cell:
                return cell["text"]
            v = cell["value"]
            return str(int(v)) if float(v).is_integer() else repr(v)
        return self._grid(formula)


class _Cell:
    def __init__(self, sheet, key):
        self.sheet, self.key = sheet, key

    def _set(self, cell):
        self.sheet.doc._check()
        self.sheet.doc._touch()
        if cell is None:
            self.sheet.cells.pop(self.key, None)
        else:
            self.sheet.cells[self.key] = cell

    def setValue(self, value):
        assert isinstance(value, float), "setValue takes a double"
        self._set({"value": value})

    def setString(self, text):
        self._set({"text": text} if text else None)

    def getFormula(self):
        cell = self.sheet.cells.get(self.key) or {}
        if "formula" in cell:
            return cell["formula"]
        return str(cell.get("text", cell.get("value", ""))) if cell else ""

    def setFormula(self, formula):
        if not formula:
            self._set(None)
        elif formula.startswith("="):
            self._set({"formula": formula, "value": 0.0})
        else:
            try:
                self._set({"value": float(formula)})
            except ValueError:
                self._set({"text": formula})


class FakeUno:
    """The `uno` module, as far as the adapter uses it."""

    def __init__(self, office):
        self.office = office

    def getComponentContext(self):
        return _LocalContext(self.office)

    @staticmethod
    def systemPathToFileUrl(path):
        assert os.path.isabs(path), "systemPathToFileUrl takes an absolute path"
        return "file://" + urllib.parse.quote(path)

    @staticmethod
    def fileUrlToSystemPath(url):
        return urllib.parse.unquote(url[len("file://"):])

    @staticmethod
    def createUnoStruct(name):
        assert name == "com.sun.star.beans.PropertyValue", name
        return PropertyValue()
