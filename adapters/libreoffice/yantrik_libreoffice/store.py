"""Putting a document on disk: `save`, `save_as` and `export_pdf`, the `Office` methods that write.

Each of them writes what the document's formulas compute into a file the caller can read back
with other tools, so each asks what `read_cells` asks (#942, review H2): before anything is
written, nothing in the document may be linked to another source and, for a spreadsheet, no
formula or named expression may reach outside it; after the write, the links are asked again,
and a file written while a link appeared is taken back. A refusal only binds a caller held to
the file rule (surface.agents_files); the person's saves are theirs.
"""

import os

from .basics import CALC, DRAW, IMPRESS, KIND_NAMES, WRITER, OfficeError, absolute

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


def _remove(path):
    try:
        os.remove(path)
    except FileNotFoundError:
        pass


class HostFiles:
    """The files the person's LibreOffice writes, reached where they lie: the person's own. The
    agents' LibreOffice is given a workdir.WorkFolder instead, which never follows what the
    sandbox left in its work folder (review H1)."""

    @staticmethod
    def remove(path):
        _remove(path)

    @staticmethod
    def size(path):
        return os.path.getsize(path) if os.path.isfile(path) else None


def new_file(path, suffixes, admit):
    """A path to a file that does not exist yet, in a directory that does, that the caller may
    write (`admit("write", path)`: home_paths' write rule, for an agent)."""
    path = absolute(path)
    admit("write", path)
    if os.path.lexists(path):
        raise OfficeError("a file is already at %s, and this action never replaces one; "
                          "choose another path" % path)
    if not os.path.isdir(os.path.dirname(path)):
        raise OfficeError("there is no directory %s to write into" % os.path.dirname(path))
    if suffixes is not None and os.path.splitext(path)[1].lower() not in suffixes:
        raise OfficeError("%s must end in %s" % (path, " or ".join(sorted(suffixes))))
    return path


class Storing:
    """The writing half of `Office` (office.py), which it is mixed into."""

    def _before_store(self, doc):
        self._unlinked(doc, scan=self.kind(doc) == CALC)

    def _after_store(self, doc, undo):
        """The links asked again once the file is written; `undo()` takes the file back."""
        try:
            self._unlinked(doc)
        except OfficeError as e:
            undo()
            raise OfficeError("%s; what was written has been taken back" % e) from None

    def save(self, document=None):
        def store(desktop):
            doc = self._find(desktop, document)
            path = self.path_of(doc)
            if not path:
                raise OfficeError("`%s` has never been saved, so it has no file to save over; "
                                  "`save_as` it to a path" % self.name_of(doc))
            self._admit("write", path)
            self._writable(doc)
            self._before_store(doc)
            # An agent's save keeps what the file held, to put it back if the write is refused.
            old = self.files.read(path) if self.agents_only else None

            def undo():
                if old is not None:
                    self.files.replace(path, old)
            doc.store()
            self._after_store(doc, undo)
            return {"document": self.name_of(doc), "path": path, "modified": bool(doc.isModified())}
        return self.run(store)

    def _new_file(self, path, suffixes):
        return new_file(path, suffixes, self._admit)

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
            self._before_store(doc)
            was = self.path_of(doc)
            doc.storeAsURL(self.url_of(path),
                           (self._prop("FilterName", filter_name), self._prop("Overwrite", False)))
            self._after_store(doc, lambda: self.files.remove(path))
            return {"document": self.name_of(doc), "path": path, "was": was}
        return self.run(store)

    def export_pdf_plan(self, path, document=None):
        """Everything `export_pdf` checks before it writes: `(doc name, work)`, where `work()`
        writes the PDF and returns the answer. Split so the surface can check in its turn and
        write outside it; the document is checked in both."""
        path = self._new_file(path, {".pdf"})

        def find(desktop):
            doc = self._find(desktop, document)
            self._before_store(doc)
            return doc, self.name_of(doc), self.kind(doc)
        doc, name, kind = self.run(find)

        def work():
            def store(_desktop):
                self._before_store(doc)
                doc.storeToURL(self.url_of(path),
                               (self._prop("FilterName", PDF_FILTERS[kind]),
                                self._prop("Overwrite", False)))
                self._after_store(doc, lambda: self.files.remove(path))
            self.run(store)
            size = self.files.size(path)
            if size is None:
                raise OfficeError("LibreOffice answered, and nothing is at %s" % path)
            return {"document": name, "path": path, "bytes": size}
        return name, work
