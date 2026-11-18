"""What every part of the adapter that talks about documents shares: the kinds, the errors, the
path rule, and how a file rule's refusal becomes a sentence."""

import os

WRITER, CALC, IMPRESS, DRAW = "writer", "calc", "impress", "draw"
KIND_NAMES = {WRITER: "a Writer document", CALC: "a Calc spreadsheet",
              IMPRESS: "an Impress presentation", DRAW: "a Draw drawing"}


class OfficeError(Exception):
    """A sentence for the caller: what went wrong and what to do instead."""


class NotReachable(OfficeError):
    """LibreOffice cannot be reached at all. `short` is the half-line describe's summary uses."""

    def __init__(self, message, short):
        super().__init__(message)
        self.short = short


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


def apply_rule(rule, what, path):
    """`rule(what, path)` (surface.agents_files), its refusal as an `OfficeError`; no rule
    refuses nothing."""
    if rule is None:
        return
    try:
        rule(what, path)
    except OfficeError:
        raise
    except Exception as e:  # noqa: BLE001 - the rule's refusal is the caller's sentence
        raise OfficeError(str(e)) from None


class PathMap:
    """Where a path this adapter names is for the LibreOffice it talks to. The person's LibreOffice
    sees the same files at the same paths (no map). An agent's sees only its sandbox, where the
    adapter's work folder `outside` is the folder `inside` (sandbox.py)."""

    def __init__(self, outside=None, inside=None):
        self.outside_root = os.path.normpath(outside) if outside else None
        self.inside_root = os.path.normpath(inside) if inside else None

    def inside(self, path):
        """`path` as LibreOffice sees it. Raises `OfficeError` for one it is not shown."""
        if self.outside_root is None:
            return path
        rel = _below(path, self.outside_root)
        if rel is None:
            raise OfficeError("%s is not among the files this LibreOffice is shown" % path)
        return os.path.join(self.inside_root, rel) if rel else self.inside_root

    def outside(self, path):
        """A path LibreOffice reports, as this adapter names it; None for one outside its share."""
        if self.outside_root is None or path is None:
            return path
        rel = _below(path, self.inside_root)
        if rel is None:
            return None
        return os.path.join(self.outside_root, rel) if rel else self.outside_root


def _below(path, root):
    """`path` relative to `root` ("" for `root` itself), or None when it is not below it."""
    path = os.path.normpath(path)
    if path == root:
        return ""
    if path.startswith(root.rstrip(os.sep) + os.sep):
        return path[len(root.rstrip(os.sep)) + 1:]
    return None
