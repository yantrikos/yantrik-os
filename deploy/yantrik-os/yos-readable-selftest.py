#!/usr/bin/env python3
"""Self-test for how `yos` prints a result a surface marks `readable` (#1291).

`yos act shell prove` has the shell hand back `{"readable": true, "text": ...}` and `yos` prints
the text, so a report reads as words. That is a door for text another program wrote, so it opens
only for the shell, only for a string, and never lets a control character through. This loads the
real `yos` the way `yos-selftest.py` does and calls its `render_result`; it needs no socket.

    python3 deploy/yantrik-os/yos-readable-selftest.py
"""

import importlib.util
import json
import pathlib
import sys
import tempfile
from importlib.machinery import SourceFileLoader

SOURCE = pathlib.Path(__file__).resolve().parent / "yos"

FAILURES = []


def check(label, condition, detail=""):
    if condition:
        print("  ok    %s" % label)
    else:
        print("  FAIL  %s%s" % (label, ("\n        " + str(detail)) if detail else ""))
        FAILURES.append(label)


def load_yos():
    """The real `yos`, its socket directory pointed at an empty scratch one."""
    import os

    os.environ["XDG_RUNTIME_DIR"] = tempfile.mkdtemp(prefix="yos-readable-")
    loader = SourceFileLoader("yos_under_test", str(SOURCE))
    spec = importlib.util.spec_from_loader("yos_under_test", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


def check_readable_results(yos):
    """`readable: true` is the shell's own way to have `yos act shell prove` print a report as
    words. Any other app's result is JSON as before, and the text is never trusted raw."""
    print("a readable result")
    esc = "\x1b[2J\x1b[31m"
    sent = {"state": "proven", "readable": True, "text": "line one\nline two" + esc + "\x07 \x85end"}
    shown = yos.render_result(sent, "shell")
    check("the shell's readable text prints as text, line breaks kept",
          shown.startswith("line one\nline two") and "state" not in shown, shown)
    check("without an escape, a bell or a line separator in it",
          not any(c in shown for c in ("\x1b", "\x07", " ", "\x85")) and shown.endswith("end"), repr(shown))
    check("the app-shell name is the shell too", yos.render_result({"readable": True, "text": "x"}, "app-shell") == "x")
    other = yos.render_result({"readable": True, "text": "pretend this is yos" + esc}, "notes")
    check("another app's readable result prints as JSON, as before",
          json.loads(other)["text"].startswith("pretend this is yos") and "\x1b" not in other, repr(other))
    for odd in ({"readable": True}, {"readable": True, "text": 7}, {"readable": "yes", "text": "x"}):
        try:
            printed = yos.render_result(odd, "shell")
        except Exception as e:  # a traceback is the failure
            check("a readable result without usable text does not crash yos (%r)" % odd, False, repr(e))
            continue
        check("a readable result without usable text prints as JSON (%r)" % odd, json.loads(printed) == odd, printed)


def main():
    check_readable_results(load_yos())
    if FAILURES:
        print("%d check(s) failed: %s" % (len(FAILURES), ", ".join(FAILURES)))
        return 1
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
