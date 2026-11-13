"""A page's own words as lines the surface keeps: what it wrote to its console, the exceptions
nobody caught, and what the browser says about it (#1152; a part of cdp.py, moved out of it)."""

import json

CONSOLE_LINE = 600         # characters kept of one line


def _line(text, cap=CONSOLE_LINE):
    """One line of a page's words: whitespace (newlines included) folded to single spaces, other
    control characters dropped, and cut at `cap`. A page's console is the page's to write, and a
    newline in it would start a line of its own in whatever reads this back — a fake fence, a
    fake instruction. So nothing a page writes ever reaches a reader as more than one line."""
    text = "".join(c for c in str(text or "") if c.isprintable() or c in " \t\n\r")
    text = " ".join(text.split())
    return text if len(text) <= cap else text[:cap - 1] + "…"


def _place(url, line, column):
    """`url:line:col` as a person reads it (DevTools counts from 0; editors from 1)."""
    if not url and line is None:
        return ""
    out = str(url or "")
    if isinstance(line, int):
        out += ":%d" % (line + 1)
        if isinstance(column, int):
            out += ":%d" % (column + 1)
    return out


def _value_of(arg):
    """One console argument (a RemoteObject) as text."""
    if "value" in arg:
        v = arg["value"]
        return v if isinstance(v, str) else json.dumps(v)
    if arg.get("unserializableValue"):
        return str(arg["unserializableValue"])
    return str(arg.get("description") or arg.get("type") or "")


def exception_entry(details):
    """A `Runtime.exceptionThrown`: "Uncaught SyntaxError: Unexpected token ';'" and where."""
    exc = details.get("exception") or {}
    described = str(exc.get("description") or "").splitlines()
    head = described[0] if described else str(exc.get("value") or "")
    text = " ".join(w for w in (str(details.get("text") or "").strip(), head) if w)
    url, line, col = details.get("url"), details.get("lineNumber"), details.get("columnNumber")
    frames = (details.get("stackTrace") or {}).get("callFrames") or []
    if not url and frames:
        url, line, col = frames[0].get("url"), frames[0].get("lineNumber"), frames[0].get("columnNumber")
    return {"level": "error", "kind": "exception", "text": _line(text or "an uncaught exception"),
            "at": _line(_place(url, line, col), 300)}


CONSOLE_LEVELS = {"error": "error", "assert": "error", "warning": "warning"}


def console_entry(p):
    """A `Runtime.consoleAPICalled`: console.error / warn / log / info / debug and the rest, or
    None for the calls that say nothing (clear, group ends, timers, profiles)."""
    kind = str(p.get("type") or "log")
    if kind in ("clear", "endGroup", "startGroup", "startGroupCollapsed", "profile", "profileEnd",
                "timeEnd", "count"):
        return None
    text = " ".join(_value_of(a) for a in (p.get("args") or []) if isinstance(a, dict))
    frames = (p.get("stackTrace") or {}).get("callFrames") or []
    at = _place(frames[0].get("url"), frames[0].get("lineNumber"), frames[0].get("columnNumber")) if frames else ""
    return {"level": CONSOLE_LEVELS.get(kind, "log"), "kind": "console." + kind,
            "text": _line(text), "at": _line(at, 300)}


def log_entry(entry):
    """A `Log.entryAdded`: the browser's own report about the page — a resource that failed to
    load (404), a security or deprecation notice — with the resource it is about."""
    level = str(entry.get("level") or "info")
    text = str(entry.get("text") or "")
    source = str(entry.get("source") or "browser")
    at = _place(entry.get("url"), entry.get("lineNumber"), None)
    return {"level": {"error": "error", "warning": "warning"}.get(level, "log"),
            "kind": "browser." + source, "text": _line(text), "at": _line(at, 300)}
