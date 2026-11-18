"""Cell references and numbers: A1 names, ranges, and how a cell's number is told."""

import re


# The most cells one read or one write carries: enough for any table a mind should be reading
# whole, small enough that an answer stays an answer.
MAX_CELLS = 10000
# Past this many cells a sheet's content is not fingerprinted on every describe.
FINGERPRINT_CELLS = 50000

_CELL = re.compile(r"\$?([A-Za-z]{1,3})\$?([1-9][0-9]{0,6})")


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


def number(value):
    """A cell's number as JSON would like it: 3 for 3.0."""
    if isinstance(value, float) and value.is_integer() and abs(value) < 2 ** 53:
        return int(value)
    return value
