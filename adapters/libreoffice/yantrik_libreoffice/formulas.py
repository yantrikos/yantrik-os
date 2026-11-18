"""Which formulas an agent may write into a cell (#942's review).

A Calc formula can read a file or a URL with the person's rights: an external reference like
`='file:///…'#$Sheet1.A1`, `INDIRECT`, `DDE`, `WEBSERVICE`, `HYPERLINK`, and `read_cells` hands
the result back. So an agent's formula is held to an allowlist: every function it calls must be
one that only reads cells of the same document, and it may name no other document. Anything
else is refused with a sentence. The person's own formulas are not held to this.
"""

import re

# Functions that compute from their arguments and the document's own cells, and nothing else.
SAFE_FUNCTIONS = frozenset("""
ABS ACOS ACOSH ASIN ASINH ATAN ATAN2 ATANH AVERAGE AVERAGEA AVERAGEIF AVERAGEIFS CEILING CHAR
CHOOSE CLEAN CODE COLUMN COLUMNS CONCAT CONCATENATE COS COSH COUNT COUNTA COUNTBLANK COUNTIF
COUNTIFS DATE DATEDIF DATEVALUE DAY DAYS DEGREES EDATE EOMONTH EVEN EXACT EXP FACT FALSE FIND
FLOOR FV HLOOKUP HOUR IF IFERROR IFNA IFS INDEX INT IPMT ISBLANK ISERROR ISEVEN ISNA ISNUMBER
ISODD ISTEXT LARGE LEFT LEN LN LOG LOG10 LOOKUP LOWER MATCH MAX MAXA MAXIFS MEDIAN MID MIN MINA
MINIFS MINUTE MOD MODE MONTH N NETWORKDAYS NOT NOW NPER NPV ODD OR AND PERCENTILE PI PMT POWER
PRODUCT PROPER PV QUARTILE QUOTIENT RADIANS RAND RANDBETWEEN RANK RATE REPLACE REPT RIGHT ROUND
ROUNDDOWN ROUNDUP ROW ROWS SEARCH SECOND SIGN SIN SINH SMALL SQRT STDEV STDEVP SUBSTITUTE SUBTOTAL
SUM SUMIF SUMIFS SUMPRODUCT SUMSQ SWITCH T TAN TANH TEXT TEXTJOIN TIME TODAY TRIM TRUE TRUNC UPPER
VALUE VAR VARP VLOOKUP WEEKDAY WEEKNUM XOR YEAR
""".split())

# The allowlist and Calc agree on what a function is: a full-width name is not one to either, so
# it is text or #NAME? and never evaluates. `+`/`@` begin a formula in the UI, but `write_cells`
# stores them as text, the safe direction.
_STRING = re.compile(r'"(?:[^"]|"")*"')
_CALL = re.compile(r"([A-Za-z_][A-Za-z0-9_.]*)\s*\(")
# A sheet of another document is named in quotes and ended with `#`; `[` opens an Excel-style
# external workbook; `://` and `file:` are a URL, and `~` is LibreOffice's reference-of-references.
_EXTERNAL = re.compile(r"'[^']*'\s*#|\[|\]|://|file:|\$'|~|@", re.IGNORECASE)


def refusal(formula):
    """The sentence that refuses `formula`, or None when an agent may write it."""
    # Text in quotes is data to the allowed functions; none of them looks a name up as a file.
    code = _STRING.sub('""', formula)
    if '"' in code.replace('""', ""):
        return "that formula has an unclosed quote"
    if _EXTERNAL.search(code):
        return ("a formula an agent writes may not name another document, a link or a URL; "
                "use values from this document's own cells")
    for name in _CALL.findall(code):
        base = name.upper()
        if base.startswith("_XLFN."):
            base = base[len("_XLFN."):]
        if base not in SAFE_FUNCTIONS:
            return ("%s() is not a function an agent may write into a cell: it could read a "
                    "file or the network, or it is not one this adapter lists as safe"
                    % name.upper())
    return None


def linked(doc):
    """The names of the other documents or sources `doc` is linked to, empty when none.

    A cell can come to hold a link without the formula showing it, so this is asked of the
    document itself after an agent writes and before an agent reads."""
    names = []
    for attribute in ("ExternalDocLinks", "DDELinks"):
        try:
            names += list(getattr(doc, attribute).getElementNames())
        except Exception:  # noqa: BLE001 - a document of another kind has no such links
            continue
    return names
