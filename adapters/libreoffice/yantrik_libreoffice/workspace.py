"""What an agent's calls reach: the sandboxed LibreOffice, working on copies of the person's files
(#953). Every agent action of the surface is a method here with the same name and arguments as
`Office`'s, so the surface serves an agent from this exactly as it serves the person from theirs.

An agent names the person's files. The sandboxed LibreOffice knows only the copies in its work
folder (copies.py). This file keeps which copy is which file, turns the agent's paths into copies'
on the way in and copies' paths into the person's on the way out, and is the one place an agent's
work reaches the person's folders:

  * `open` holds the file to the rule, refuses one the person has open in their own LibreOffice
    (or with a LibreOffice lock beside it, as written or where it resolves: #942 L4), and copies
    it in;
  * `save` holds the file to the write rule again, refuses when the person has opened it since or
    it changed on disk, lets the sandboxed LibreOffice save the copy (the scan and the link checks
    run there, as before), and only then puts the copy's bytes over the person's file;
  * `save_as` and `export_pdf` write into the work folder, then create the person's new file, never
    replacing one.

The formula scan and link checks of guard.py run in the sandboxed `Office`, as a second wall: what
a document computes in the sandbox cannot read the person's files, and what it computes is still
checked before it is saved, exported or handed back.
"""

import os

from . import copies
from .basics import OfficeError, absolute, apply_rule
from .store import SAVE_FILTERS, new_file

HELD = ("the person has %s open in their own LibreOffice, and an agent works only on a copy in a "
        "LibreOffice of its own; ask the person to close it first, so neither overwrites the "
        "other's work")
NOT_THEIRS = ("`%s` is not a copy of one of the person's files (it was never saved, or its save "
              "did not reach the person's folder); `save_as` it to a path")


def lock_file(path):
    """The file LibreOffice keeps beside a document it has open for editing."""
    return os.path.join(os.path.dirname(path), ".~lock.%s#" % os.path.basename(path))


class AgentWorkspace:
    """The agents' side of the surface, over `office`: the sandboxed `Office`, whose paths are the
    copies in `work`, the sandbox's work folder (a workdir.WorkFolder, the only way this side
    reaches into it). `admit` is the surface's rule for the person's files; `person` is the
    person's own `Office`, asked which files are open there."""

    def __init__(self, office, work, admit=None, person=None):
        self.office = office
        self.work = work
        self.admit = admit
        self.person = person
        self._copies = {}       # copy's path -> copies.Copy

    # ── the person's paths and the copies' ──────────────────────────────────

    def _rule(self, what, path):
        """The rule for `path` as named, and where it resolves when that is somewhere else."""
        apply_rule(self.admit, what, path)
        real = os.path.realpath(path)
        if real != path:
            apply_rule(self.admit, what, real)
        return real

    def _inward(self, document):
        """`document` as the sandboxed LibreOffice knows it: a path of the person's becomes its
        copy's; a name is a name."""
        if not isinstance(document, str) or not (os.sep in document or document.startswith("~")):
            return document
        wanted = os.path.normpath(os.path.expanduser(document.strip()))
        for copy in self._copies.values():
            if wanted in (copy.given, copy.real):
                return copy.work
        return document

    def _person_path(self, path):
        copy = self._copies.get(path)
        return copy.given if copy is not None else path

    def _outward(self, answer):
        """An answer with the copies' paths turned back into the person's."""
        if isinstance(answer, dict):
            for key in ("path", "was"):
                if isinstance(answer.get(key), str):
                    answer[key] = self._person_path(answer[key])
        return answer

    def _copy_of(self, document):
        path = self.office.path_for(self._inward(document))
        copy = self._copies.get(path)
        if copy is None:
            raise OfficeError(NOT_THEIRS % (os.path.basename(path) if path else document))
        return copy

    def _held_by_person(self, *paths):
        for path in paths:
            if os.path.lexists(lock_file(path)):
                return True
        if self.person is None:
            return False
        try:
            theirs = self.person.open_paths()
        except OfficeError:  # the person's LibreOffice is not running: nothing of theirs is open
            return False
        return any(_same(mine, p) for mine in paths for p in theirs)

    def _forget_closed(self):
        """Drop the copies no document in the sandbox has open any more."""
        try:
            open_now = set(self.office.open_paths())
        except OfficeError:
            return
        for path in [p for p in self._copies if p not in open_now]:
            del self._copies[path]
            self.work.discard(os.path.dirname(path))

    # ── the actions ─────────────────────────────────────────────────────────

    def open(self, path, read_only=False):
        path = absolute(path)
        real = self._rule("read", path)
        for copy in self._copies.values():
            if copy.real == real and copy.work in self.office.open_paths():
                answer = self.office.open(copy.work, read_only)
                return self._outward(dict(answer, already_open=True))
        if self._held_by_person(path, real):
            raise OfficeError(HELD % path)
        slot = self.work.slot()
        try:
            copy = copies.take(path, real, os.path.join(slot, os.path.basename(path)), self.work)
            answer = self.office.open(copy.work, read_only)
        except Exception:
            self.work.discard(slot)
            raise
        self._copies[copy.work] = copy
        return self._outward(answer)

    def read_text(self, document=None, start=0, max_chars=20000):
        return self.office.read_text(self._inward(document), start, max_chars)

    def read_cells(self, range=None, sheet=None, document=None):  # noqa: A002 - the caller's word
        return self.office.read_cells(range, sheet, self._inward(document))

    def write_text(self, text, at="end", document=None):
        return self.office.write_text(text, at, self._inward(document))

    def write_cells(self, cells, sheet=None, document=None):
        return self.office.write_cells(cells, sheet, self._inward(document))

    def close(self, document=None):
        answer = self.office.close(self._inward(document))
        self._forget_closed()
        return answer

    def save(self, document=None):
        copy = self._copy_of(document)
        self._rule("write", copy.given)
        if self._held_by_person(copy.given, copy.real):
            raise OfficeError(HELD % copy.given)
        if not copies.unchanged(copy):
            raise OfficeError(copies.CHANGED % copy.given)
        answer = self.office.save(copy.work)
        copies.put_back(copy, self.work.read(copy.work))
        return self._outward(answer)

    def _target(self, path, suffixes):
        """The person's new file: the write rule as named and where its folder resolves."""
        path = new_file(path, suffixes, lambda what, p: apply_rule(self.admit, what, p))
        folder = os.path.realpath(os.path.dirname(path))
        real = os.path.join(folder, os.path.basename(path))
        if real != path:
            new_file(real, suffixes, lambda what, p: apply_rule(self.admit, what, p))
        return path, folder, os.path.basename(path)

    def save_as(self, path, document=None):
        shown, folder, name = self._target(path, SAVE_FILTERS)
        was = self._inward(document)
        before = self.office.path_for(was)
        out = os.path.join(self.work.slot(), name)
        answer = self.office.save_as(out, was)
        data = self.work.read(out)
        mode = copies.create(data, folder, name, shown)
        self._copies[out] = copies.Copy(shown, os.path.join(folder, name), out,
                                        copies.digest_of(data), mode)
        was = self._person_path(before) if before else None
        self._forget_closed()
        return dict(answer, path=shown, was=was)

    def export_pdf_plan(self, path, document=None):
        shown, folder, name = self._target(path, {".pdf"})
        slot = self.work.slot()
        out = os.path.join(slot, name)
        try:
            doc_name, work = self.office.export_pdf_plan(out, self._inward(document))
        except Exception:
            self.work.discard(slot)
            raise

        def export():
            try:
                work()
                data = self.work.read(out)
                copies.create(data, folder, name, shown)
            finally:
                self.work.discard(slot)
            return {"document": doc_name, "path": shown, "bytes": len(data)}
        return doc_name, export

    # ── what describe shows ─────────────────────────────────────────────────

    def overview(self):
        documents, front = self.office.overview()
        return [self._outward(d) for d in documents], front

    def open_paths(self):
        return [self._person_path(p) for p in self.office.open_paths()]


def _same(a, b):
    try:
        return os.path.samefile(a, b)
    except OSError:
        return os.path.normpath(a) == os.path.normpath(b)
